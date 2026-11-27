package app.yuxino.mimi.android.captions

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RectF
import android.os.Build
import android.os.Bundle
import android.view.View
import android.view.accessibility.AccessibilityNodeInfo
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.localization.InterfaceLanguage
import kotlin.math.roundToInt

/**
 * Compact vertical background-opacity slider drawn inside the panel's 16dp left margin.
 * Painted visuals stay strictly within x = 1.5dp..14.5dp so they never cover caption glyphs.
 */
internal class CaptionOpacitySlider(context: Context) : View(context) {
    var progress: Int = 80
        set(value) {
            val clamped = value.coerceIn(0, 100)
            if (field != clamped) {
                field = clamped
                if (Build.VERSION.SDK_INT >= 30) stateDescription = "$clamped%"
                invalidate()
            }
        }

    var lightSurface: Boolean = false
        set(value) {
            if (field != value) {
                field = value
                invalidate()
            }
        }

    var dragging: Boolean = false
        set(value) {
            if (field != value) {
                field = value
                invalidate()
            }
        }

    var onAccessibilityStep: ((Int) -> Unit)? = null

    private val trackPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.FILL }
    private val trackOutlinePaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        style = Paint.Style.STROKE
        strokeWidth = dpF(0.75f)
    }
    private val fillPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.FILL }
    private val thumbPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.FILL }
    private val strokePaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        style = Paint.Style.STROKE
        strokeWidth = dpF(1f)
    }
    private val trackRect = RectF()
    private val fillRect = RectF()

    init {
        tag = "overlay-opacity-slider"
        isClickable = true
        isFocusable = true
        contentDescription = InterfaceLanguage.context(context).getString(R.string.settings_background)
        if (Build.VERSION.SDK_INT >= 30) stateDescription = "$progress%"
        accessibilityDelegate = object : AccessibilityDelegate() {
            override fun onInitializeAccessibilityNodeInfo(host: View, info: AccessibilityNodeInfo) {
                super.onInitializeAccessibilityNodeInfo(host, info)
                info.rangeInfo = if (Build.VERSION.SDK_INT >= 30) {
                    AccessibilityNodeInfo.RangeInfo(
                        AccessibilityNodeInfo.RangeInfo.RANGE_TYPE_PERCENT,
                        0f,
                        100f,
                        progress.toFloat(),
                    )
                } else {
                    @Suppress("DEPRECATION")
                    AccessibilityNodeInfo.RangeInfo.obtain(
                        AccessibilityNodeInfo.RangeInfo.RANGE_TYPE_PERCENT,
                        0f,
                        100f,
                        progress.toFloat(),
                    )
                }
                if (isEnabled && isClickable) {
                    if (progress < 100) info.addAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_FORWARD)
                    if (progress > 0) info.addAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_BACKWARD)
                    info.addAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_SET_PROGRESS)
                }
            }

            override fun performAccessibilityAction(host: View, action: Int, args: Bundle?): Boolean {
                if (!isEnabled || !isClickable) return super.performAccessibilityAction(host, action, args)
                val next = when (action) {
                    AccessibilityNodeInfo.ACTION_SCROLL_FORWARD -> (progress + 10).coerceIn(0, 100)
                    AccessibilityNodeInfo.ACTION_SCROLL_BACKWARD -> (progress - 10).coerceIn(0, 100)
                    AccessibilityNodeInfo.AccessibilityAction.ACTION_SET_PROGRESS.id ->
                        args?.getFloat(AccessibilityNodeInfo.ACTION_ARGUMENT_PROGRESS_VALUE, progress.toFloat())
                            ?.roundToInt()?.coerceIn(0, 100) ?: return false
                    else -> return super.performAccessibilityAction(host, action, args)
                }
                if (next != progress) {
                    progress = next
                    onAccessibilityStep?.invoke(next)
                }
                return true
            }
        }
    }

    /** Maps a local y coordinate inside this view to 0 (bottom/transparent) .. 100 (top/solid). */
    fun valueFromY(localY: Float): Int {
        val top = trackTop()
        val bottom = trackBottom()
        val span = (bottom - top).coerceAtLeast(1f)
        val fraction = 1f - ((localY - top) / span).coerceIn(0f, 1f)
        return (fraction * 100f).roundToInt().coerceIn(0, 100)
    }

    private fun trackTop(): Float = dpF(8f)
    private fun trackBottom(): Float = (height - dpF(8f)).coerceAtLeast(trackTop() + 1f)

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        val centerX = dpF(8f)
        val halfTrack = dpF(1.5f)
        val thumbRadius = dpF(if (dragging) 6f else 5f)
        val top = trackTop()
        val bottom = trackBottom()
        val span = (bottom - top).coerceAtLeast(1f)
        val thumbY = bottom - span * (progress / 100f)

        if (lightSurface) {
            trackPaint.color = 0x38000000
            trackOutlinePaint.color = 0x55FFFFFF
            fillPaint.color = 0xCC141414.toInt()
            thumbPaint.color = 0xFF141414.toInt()
            strokePaint.color = 0x77FFFFFF
        } else {
            trackPaint.color = 0x44FFFFFF
            trackOutlinePaint.color = 0x66000000
            fillPaint.color = 0xE6FFFFFF.toInt()
            thumbPaint.color = Color.WHITE
            strokePaint.color = 0x77000000
        }

        trackRect.set(centerX - halfTrack, top, centerX + halfTrack, bottom)
        canvas.drawRoundRect(trackRect, halfTrack, halfTrack, trackPaint)
        canvas.drawRoundRect(trackRect, halfTrack, halfTrack, trackOutlinePaint)

        fillRect.set(centerX - halfTrack, thumbY, centerX + halfTrack, bottom)
        canvas.drawRoundRect(fillRect, halfTrack, halfTrack, fillPaint)

        canvas.drawCircle(centerX, thumbY, thumbRadius, thumbPaint)
        canvas.drawCircle(centerX, thumbY, thumbRadius, strokePaint)
    }

    private fun dpF(value: Float): Float = value * resources.displayMetrics.density

    companion object {
        const val TOUCH_WIDTH_DP = 24
        const val VISUAL_CENTER_X_DP = 8
    }
}
