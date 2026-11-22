package app.yuxino.mimi.android.shortcuts

import kotlin.math.roundToInt

internal enum class CaptionEdge { LEFT, RIGHT }

/** Fractions of the space the whole touch target can travel, inside the fitted display. */
internal data class CaptionEdgePlacement(val x: Float = 1f, val y: Float = 0.5f) {
    fun left(travel: Int): Int = (x * travel).roundToInt()
    fun top(travel: Int): Int = (y * travel).roundToInt()

    fun moved(left: Int, top: Int, travelX: Int, travelY: Int) = CaptionEdgePlacement(
        if (travelX > 0) (left.toFloat() / travelX).coerceIn(0f, 1f) else x,
        if (travelY > 0) (top.toFloat() / travelY).coerceIn(0f, 1f) else y,
    )

    fun nearestSide() = if (x < .5f) CaptionEdge.LEFT else CaptionEdge.RIGHT

    fun dock(edge: CaptionEdge) = copy(x = if (edge == CaptionEdge.LEFT) 0f else 1f)
}

/** Once a press crosses touch slop it stays a drag, including when it returns to its origin. */
internal class CaptionEdgeGesture(
    private val rawX: Float, private val rawY: Float, private val slop: Int,
    private val windowLeft: Int, private val windowTop: Int,
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

    fun isTap(heldMs: Long, longPressMs: Int, inside: Boolean) =
        !dragging && inside && heldMs < longPressMs
}
