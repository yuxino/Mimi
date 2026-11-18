package app.yuxino.mimi.android.captions

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.widget.TextView

/** Optional faint sentence boundaries; blank-line spacing remains when disabled. */
@SuppressLint("AppCompatCustomView")
internal class CaptionHistoryTextView(context: Context) : TextView(context) {
    var dividers = false
        set(value) { if (field != value) { field = value; invalidate() } }
    private val rule = Paint().apply { color = 0xFF555555.toInt(); strokeWidth = resources.displayMetrics.density }
    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        if (!dividers) return
        val layout = layout ?: return
        for (line in 1 until minOf(layout.lineCount, maxLines)) {
            if (text.subSequence(layout.getLineStart(line), layout.getLineEnd(line)).isBlank()) {
                val y = (layout.getLineTop(line) + layout.getLineBottom(line)) / 2f + compoundPaddingTop
                canvas.drawLine(compoundPaddingLeft.toFloat(), y, (width - compoundPaddingRight).toFloat(), y, rule)
            }
        }
    }
}
