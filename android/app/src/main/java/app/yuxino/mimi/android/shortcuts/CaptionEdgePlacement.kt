package app.yuxino.mimi.android.shortcuts

import kotlin.math.roundToInt

internal enum class CaptionEdge { LEFT, RIGHT, TOP, BOTTOM }

/** Fractions of the space the whole touch target can travel, inside the fitted display. */
internal data class CaptionEdgePlacement(val x: Float = 1f, val y: Float = 0.5f) {
    fun left(travel: Int): Int = (x * travel).roundToInt()
    fun top(travel: Int): Int = (y * travel).roundToInt()

    fun moved(left: Int, top: Int, travelX: Int, travelY: Int) = CaptionEdgePlacement(
        if (travelX > 0) (left.toFloat() / travelX).coerceIn(0f, 1f) else x,
        if (travelY > 0) (top.toFloat() / travelY).coerceIn(0f, 1f) else y,
    )

    fun nearest(travelX: Int, travelY: Int): CaptionEdge {
        val left = left(travelX)
        val top = top(travelY)
        val right = travelX - left
        val bottom = travelY - top
        return when (minOf(left, right, top, bottom)) {
            left -> CaptionEdge.LEFT
            right -> CaptionEdge.RIGHT
            top -> CaptionEdge.TOP
            else -> CaptionEdge.BOTTOM
        }
    }

    fun dock(edge: CaptionEdge) = when (edge) {
        CaptionEdge.LEFT -> copy(x = 0f)
        CaptionEdge.RIGHT -> copy(x = 1f)
        CaptionEdge.TOP -> copy(y = 0f)
        CaptionEdge.BOTTOM -> copy(y = 1f)
    }
}

/** Once a press crosses touch slop it stays a drag, including when it returns to its origin. */
internal class CaptionEdgeGesture(
    private val rawX: Float, private val rawY: Float, private val slop: Int,
    private val windowLeft: Int, private val windowTop: Int, private val startedTucked: Boolean,
) {
    var dragging = false
        private set

    fun update(x: Float, y: Float): Boolean {
        val dx = x - rawX
        val dy = y - rawY
        if (dx * dx + dy * dy > slop.toFloat() * slop) dragging = true
        return dragging
    }

    fun left(x: Float) = windowLeft + (x - rawX).roundToInt()
    fun top(y: Float) = windowTop + (y - rawY).roundToInt()

    fun activates(heldMs: Long, longPressMs: Int, inside: Boolean) =
        !startedTucked && !dragging && inside && heldMs < longPressMs
}
