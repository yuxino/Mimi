package app.yuxino.mimi.android.shortcuts

import android.animation.ValueAnimator
import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.view.View
import android.view.animation.LinearInterpolator
import app.yuxino.mimi.android.R
import kotlin.math.abs
import kotlin.math.roundToInt
import kotlin.math.roundToLong

/** A single faceplate stays still while the ribbon gives way to the available action. */
internal class CaptionEdgeArtwork(context: Context) : View(context) {
    private val density = resources.displayMetrics.density
    private val surface = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.rgb(39, 42, 47) }
    private val rim = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.rgb(98, 101, 107) }
    private val ribbon = requireNotNull(context.getDrawable(R.drawable.ic_edge_ribbon)).mutate()
    private val start = actionIcon(R.drawable.ic_edge_start)
    private val stop = actionIcon(R.drawable.ic_edge_stop)
    private var action = start
    private var dockX = 0f
    private var dockY = 0f
    private var actionVisible = true
    var actionOpacity = 1f
        private set
    private val fade = ValueAnimator().apply {
        interpolator = LinearInterpolator()
        addUpdateListener { actionOpacity = it.animatedValue as Float; invalidate() }
    }

    init {
        importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO
        ribbon.setBounds(-9, -7, 9, 7)
    }

    private fun actionIcon(resource: Int) = requireNotNull(context.getDrawable(resource)).mutate().apply {
        setTint(Color.rgb(243, 244, 246))
        setBounds(-7, -7, 7, 7)
    }

    fun place(left: Int, right: Int, top: Int, bottom: Int) {
        // Only the last few dp of travel shape the flat edge. Between edges the mark follows
        // the finger in the center of the hit target, including when it crosses the midline.
        fun attachment(distance: Int) = (1f - distance / (16f * density)).coerceIn(0f, 1f)
        dockX = attachment(right) - attachment(left)
        dockY = attachment(bottom) - attachment(top)
        invalidate()
    }

    fun setStop(stopping: Boolean) {
        action = if (stopping) stop else start
        invalidate()
    }

    override fun onDraw(canvas: Canvas) {
        canvas.save()
        canvas.translate(width / 2f, height / 2f)
        canvas.scale(density, density)
        canvas.translate(11f * dockX, 11f * dockY)
        // Extend the far side past the display clip to join it flush to any fitted edge.
        val left = -14f + 8f * minOf(0f, dockX)
        val right = 14f + 8f * maxOf(0f, dockX)
        val top = -13f + 8f * minOf(0f, dockY)
        val bottom = 13f + 8f * maxOf(0f, dockY)
        canvas.drawRoundRect(left, top, right, bottom, 8f, 8f, rim)
        val seam = 0.6f
        canvas.drawRoundRect(left + seam, top + seam, right - seam, bottom - seam, 8f - seam, 8f - seam, surface)

        val alpha = (255 * actionOpacity).roundToInt()
        if (alpha < 255) {
            ribbon.alpha = 255 - alpha
            ribbon.draw(canvas)
        }
        if (alpha > 0) {
            action.alpha = alpha
            action.draw(canvas)
        }
        canvas.restore()
    }

    fun showAction(visible: Boolean) {
        if (visible == actionVisible) return
        actionVisible = visible
        fade.cancel()
        val target = if (visible) 1f else 0f
        if (!isAttachedToWindow || !ValueAnimator.areAnimatorsEnabled() || actionOpacity == target) {
            actionOpacity = target
            invalidate()
            return
        }
        fade.setFloatValues(actionOpacity, target)
        fade.duration = (160 * abs(target - actionOpacity)).roundToLong()
        fade.start()
    }

    override fun onDetachedFromWindow() {
        fade.cancel()
        actionOpacity = if (actionVisible) 1f else 0f
        super.onDetachedFromWindow()
    }
}
