package app.yuxino.mimi.android.shortcuts

import android.annotation.SuppressLint
import android.content.Context
import android.content.res.ColorStateList
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.RippleDrawable
import android.view.Gravity
import android.view.View
import android.widget.FrameLayout
import android.widget.ImageView
import androidx.core.graphics.ColorUtils
import app.yuxino.mimi.android.R
import kotlin.math.roundToInt

/** Visual content only. The parent owns the entire 48 dp touch and accessibility target. */
internal class CaptionEdgeArtwork(context: Context) : FrameLayout(context) {
    private val ink = context.getColor(R.color.mimi_on_primary)
    private val surface = context.getColor(R.color.mimi_primary)
    private val plate = GradientDrawable().apply {
        setColor(surface)
        setStroke(dp(1), context.getColor(R.color.mimi_muted))
    }
    private val logo = ImageView(context).apply {
        setImageResource(R.drawable.mimi_brand_small)
        scaleType = ImageView.ScaleType.FIT_CENTER
    }
    private val action = ImageView(context).apply {
        background = GradientDrawable().apply {
            shape = GradientDrawable.OVAL
            setColor(surface)
            setStroke(dp(1), ink)
        }
        imageTintList = ColorStateList.valueOf(ink)
        scaleType = ImageView.ScaleType.FIT_CENTER
        setPadding(dp(3), dp(3), dp(3), dp(3))
    }

    init {
        importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
        isDuplicateParentStateEnabled = true
        foreground = RippleDrawable(ColorStateList.valueOf(ColorUtils.setAlphaComponent(ink, 55)), null,
            GradientDrawable().apply { shape = GradientDrawable.OVAL; setColor(ink) })
        addView(logo)
        addView(action, LayoutParams(dp(16), dp(16), Gravity.BOTTOM or Gravity.END))
    }

    fun setStop(stop: Boolean) {
        action.setImageResource(if (stop) R.drawable.ic_edge_stop else R.drawable.ic_edge_start)
    }

    @SuppressLint("RtlHardcoded") // Dock against the physical display edge selected by the drag.
    fun place(edge: CaptionEdge?) {
        val side = edge == CaptionEdge.LEFT || edge == CaptionEdge.RIGHT
        val width = if (edge == null) 38 else if (side) 30 else 36
        val height = if (edge == null) 38 else if (side) 36 else 30
        val alignment = when (edge) {
            CaptionEdge.LEFT -> Gravity.LEFT or Gravity.CENTER_VERTICAL
            CaptionEdge.RIGHT -> Gravity.RIGHT or Gravity.CENTER_VERTICAL
            CaptionEdge.TOP -> Gravity.TOP or Gravity.CENTER_HORIZONTAL
            CaptionEdge.BOTTOM -> Gravity.BOTTOM or Gravity.CENTER_HORIZONTAL
            null -> Gravity.CENTER
        }
        layoutParams = LayoutParams(dp(width), dp(height), alignment)
        val radius = dp(if (edge == null) 19 else 15).toFloat()
        fun corner(a: CaptionEdge, b: CaptionEdge) = if (edge == a || edge == b) 0f else radius
        val tl = corner(CaptionEdge.TOP, CaptionEdge.LEFT)
        val tr = corner(CaptionEdge.TOP, CaptionEdge.RIGHT)
        val br = corner(CaptionEdge.BOTTOM, CaptionEdge.RIGHT)
        val bl = corner(CaptionEdge.BOTTOM, CaptionEdge.LEFT)
        plate.cornerRadii = floatArrayOf(tl, tl, tr, tr, br, br, bl, bl)
        // The launcher art includes transparent padding. Keep the same complete mark at both sizes.
        val logoSize = dp(if (edge == null) 44 else 34)
        logo.layoutParams = LayoutParams(logoSize, logoSize, Gravity.CENTER)
        background = if (edge == null) null else plate
        action.visibility = if (edge == null) VISIBLE else GONE
        alpha = if (edge == null) 1f else 0.8f
    }

    private fun dp(value: Int) = (value * resources.displayMetrics.density).roundToInt()
}
