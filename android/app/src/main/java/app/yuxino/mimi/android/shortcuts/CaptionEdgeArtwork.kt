package app.yuxino.mimi.android.shortcuts

import android.animation.ValueAnimator
import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.BitmapShader
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Matrix
import android.graphics.Paint
import android.graphics.RectF
import android.graphics.Shader
import android.view.View
import android.view.animation.LinearInterpolator
import app.yuxino.mimi.android.R
import kotlin.math.abs
import kotlin.math.ceil
import kotlin.math.roundToInt
import kotlin.math.roundToLong

/** One portrait and one touch target. The paper tab beside the portrait reveals its current action. */
internal class CaptionEdgeArtwork(context: Context) : View(context) {
    private val density = resources.displayMetrics.density
    private val radius = 15f * density
    private val inset = 2f * density
    private val hairline = 0.65f * density
    private val paper = Paint(Paint.ANTI_ALIAS_FLAG)
    private val rimColor = Color.rgb(227, 223, 226)
    private val portrait = Paint(Paint.ANTI_ALIAS_FLAG or Paint.FILTER_BITMAP_FLAG).apply {
        // Frame the face and hair bow from the same artwork used by the Android launcher.
        val artwork = BitmapFactory.decodeResource(resources, R.drawable.mimi_brand)
        val cropSize = artwork.width / 2
        var image = Bitmap.createBitmap(artwork, artwork.width / 4, (artwork.height * .17f).roundToInt(), cropSize, cropSize)
        val pixels = ceil(2 * radius).toInt()
        // A single bilinear draw only samples four texels. Average the fine hair lines
        // in half-size steps, then let the shader perform the final resize on screen.
        while (image.width >= 2 * pixels) {
            image = Bitmap.createScaledBitmap(image, image.width / 2, image.height / 2, true)
        }
        val source = RectF(0f, 0f, image.width.toFloat(), image.height.toFloat())
        shader = BitmapShader(image, Shader.TileMode.CLAMP, Shader.TileMode.CLAMP).apply {
            setLocalMatrix(Matrix().apply {
                setRectToRect(source, RectF(-radius, -radius, radius, radius), Matrix.ScaleToFit.FILL)
            })
        }
    }
    private val start = requireNotNull(context.getDrawable(R.drawable.ic_edge_start)).mutate().apply { setTint(Color.DKGRAY) }
    private val stop = requireNotNull(context.getDrawable(R.drawable.ic_edge_stop)).mutate().apply { setTint(Color.DKGRAY) }
    private var action = start
    private var placement = CaptionEdgePlacement()
    private var edge = CaptionEdge.RIGHT
    private var nearestEdge = CaptionEdge.RIGHT
    private var actionVisible = true
    var actionOpacity = 1f
        private set
    private val fade = ValueAnimator().apply {
        interpolator = LinearInterpolator()
        addUpdateListener { actionOpacity = it.animatedValue as Float; invalidate() }
    }

    init {
        importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO
    }

    fun place(position: CaptionEdgePlacement, nearest: CaptionEdge) {
        placement = position
        nearestEdge = nearest
        if (!isAttachedToWindow || actionOpacity == 0f) edge = nearest
        invalidate()
    }

    fun setStop(stopping: Boolean) {
        action = if (stopping) stop else start
        invalidate()
    }

    override fun onDraw(canvas: Canvas) {
        val margin = radius + inset
        val x = margin + (width - 2 * margin) * placement.x
        val y = margin + (height - 2 * margin) * placement.y
        canvas.save()
        canvas.translate(x, y)
        if (actionOpacity > 0f) {
            val room = when (edge) {
                CaptionEdge.RIGHT -> x
                CaptionEdge.LEFT -> width - x
                CaptionEdge.BOTTOM -> y
                CaptionEdge.TOP -> height - y
            }
            drawTab(canvas, minOf(30f * density, room - density))
        }
        paper.color = rimColor
        canvas.drawCircle(0f, 0f, radius, paper)
        canvas.drawCircle(0f, 0f, radius - hairline, portrait)
        canvas.restore()
    }

    private fun drawTab(canvas: Canvas, reach: Float) {
        // Rotate the paper toward the inside of the display. The play mark always points right.
        val angle = when (edge) {
            CaptionEdge.RIGHT -> 0f
            CaptionEdge.LEFT -> 180f
            CaptionEdge.BOTTOM -> 90f
            CaptionEdge.TOP -> -90f
        }
        val opacity = (255 * actionOpacity).roundToInt()
        val halfHeight = 11f * density
        canvas.save()
        canvas.rotate(angle)
        paper.color = rimColor
        paper.alpha = opacity
        canvas.drawRoundRect(-reach, -halfHeight, 0f, halfHeight, halfHeight, halfHeight, paper)
        paper.color = Color.WHITE
        paper.alpha = opacity
        val inner = halfHeight - hairline
        canvas.drawRoundRect(-reach + hairline, -inner, 0f, inner, inner, inner, paper)
        canvas.translate(-(reach + radius) / 2, 0f)
        canvas.rotate(-angle)
        val halfGlyph = (minOf(10 * density, reach - radius - density) / 2).roundToInt().coerceAtLeast(0)
        action.setBounds(-halfGlyph, -halfGlyph, halfGlyph, halfGlyph)
        action.alpha = opacity
        action.draw(canvas)
        canvas.restore()
    }

    fun showAction(visible: Boolean) {
        val turn = visible && edge != nearestEdge
        if (visible == actionVisible && !turn) return
        actionVisible = visible
        fade.cancel()
        if (turn) {
            // A drag may have reached a different edge before the old tab finished fading.
            edge = nearestEdge
            actionOpacity = 0f
        }
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
