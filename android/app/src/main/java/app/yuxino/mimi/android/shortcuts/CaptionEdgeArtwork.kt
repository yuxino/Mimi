package app.yuxino.mimi.android.shortcuts

import android.content.Context
import android.content.res.ColorStateList
import android.graphics.drawable.GradientDrawable
import android.view.Gravity
import android.view.View
import android.widget.FrameLayout
import android.widget.ImageView
import app.yuxino.mimi.android.R
import kotlin.math.roundToInt

/** Visual content only. The parent owns the entire 48 dp touch and accessibility target. */
internal class CaptionEdgeArtwork(context: Context) : FrameLayout(context) {
    private val ink = context.getColor(R.color.mimi_on_primary)
    private val surface = context.getColor(R.color.mimi_primary)
    private val logo = ImageView(context).apply {
        setImageResource(R.drawable.mimi_brand_small)
        scaleType = ImageView.ScaleType.FIT_CENTER
        // Use the docked opacity in both states. Only the action badge changes on touch.
        alpha = 0.8f
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
        addView(logo, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
        addView(action, LayoutParams(dp(16), dp(16), Gravity.BOTTOM or Gravity.END))
    }

    fun setStop(stop: Boolean) {
        action.setImageResource(if (stop) R.drawable.ic_edge_stop else R.drawable.ic_edge_start)
    }

    fun showAction(visible: Boolean) {
        action.visibility = if (visible) VISIBLE else INVISIBLE
    }

    private fun dp(value: Int) = (value * resources.displayMetrics.density).roundToInt()
}
