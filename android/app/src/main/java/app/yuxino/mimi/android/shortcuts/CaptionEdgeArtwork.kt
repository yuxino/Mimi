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
    private val logo = ImageView(context).apply {
        setImageResource(R.drawable.mimi_brand_small)
        scaleType = ImageView.ScaleType.FIT_CENTER
        isDuplicateParentStateEnabled = true
        // Keep press feedback inside the app icon's alpha, including when it sits against a screen edge.
        foreground = RippleDrawable(ColorStateList.valueOf(ColorUtils.setAlphaComponent(ink, 55)), null,
            context.getDrawable(R.drawable.mimi_brand_small))
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
        // The launcher art includes transparent padding. Keep the same complete mark at both sizes.
        val logoSize = dp(if (edge == null) 44 else 34)
        logo.layoutParams = LayoutParams(logoSize, logoSize, Gravity.CENTER)
        action.visibility = if (edge == null) VISIBLE else GONE
        alpha = if (edge == null) 1f else 0.8f
    }

    private fun dp(value: Int) = (value * resources.displayMetrics.density).roundToInt()
}
