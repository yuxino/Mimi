package app.yuxino.mimi.android.shortcuts

import android.content.Context
import android.graphics.Canvas
import android.view.View
import app.yuxino.mimi.android.R
import kotlin.math.roundToInt

/** Mirror the original transparent pose only when a drag has chosen its new side. */
internal class CaptionEdgeArtwork(context: Context) : View(context) {
    private val portrait = requireNotNull(context.getDrawable(R.drawable.mimi_edge_peek)).mutate()
    private val size = (30 * resources.displayMetrics.density).roundToInt()
    private val portraitWidth = (size.toFloat() * portrait.intrinsicWidth / portrait.intrinsicHeight).roundToInt()
    private var side = CaptionEdge.RIGHT

    init {
        importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO
        portrait.setBounds(0, 0, portraitWidth, size)
        // Keep the same slight translucency on press and release so the face never flashes.
        portrait.alpha = 204
    }

    fun face(edge: CaptionEdge) {
        if (edge == side) return
        side = edge
        invalidate()
    }

    override fun onDraw(canvas: Canvas) {
        canvas.save()
        canvas.translate(if (side == CaptionEdge.RIGHT) (width - portraitWidth).toFloat() else portraitWidth.toFloat(),
            (height - size) / 2f)
        if (side == CaptionEdge.LEFT) canvas.scale(-1f, 1f)
        portrait.draw(canvas)
        canvas.restore()
    }
}
