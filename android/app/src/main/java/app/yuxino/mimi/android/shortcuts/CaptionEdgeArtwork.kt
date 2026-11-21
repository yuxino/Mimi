package app.yuxino.mimi.android.shortcuts

import android.animation.ValueAnimator
import android.content.Context
import android.content.res.ColorStateList
import android.graphics.drawable.GradientDrawable
import android.view.Gravity
import android.view.View
import android.view.animation.LinearInterpolator
import android.widget.FrameLayout
import android.widget.ImageView
import app.yuxino.mimi.android.R
import kotlin.math.abs
import kotlin.math.roundToInt
import kotlin.math.roundToLong

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
    private var actionVisible = true
    private val actionFade = ValueAnimator().apply {
        interpolator = LinearInterpolator()
        addUpdateListener { action.alpha = it.animatedValue as Float }
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
        if (visible == actionVisible) return
        actionVisible = visible
        actionFade.cancel()
        val target = if (visible) 1f else 0f
        val start = action.alpha
        if (!isAttachedToWindow || !ValueAnimator.areAnimatorsEnabled() || start == target) {
            action.alpha = target
            return
        }
        // Tapping during the idle fade reverses it from the currently painted opacity.
        actionFade.setFloatValues(start, target)
        actionFade.duration = (160 * abs(target - start)).roundToLong()
        actionFade.start()
    }

    override fun onDetachedFromWindow() {
        actionFade.cancel()
        action.alpha = if (actionVisible) 1f else 0f
        super.onDetachedFromWindow()
    }

    private fun dp(value: Int) = (value * resources.displayMetrics.density).roundToInt()
}
