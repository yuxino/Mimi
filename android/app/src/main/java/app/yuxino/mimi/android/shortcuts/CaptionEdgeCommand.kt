package app.yuxino.mimi.android.shortcuts

import android.annotation.SuppressLint
import android.content.Context
import android.content.res.ColorStateList
import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.RippleDrawable
import android.text.TextUtils
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.widget.FrameLayout
import android.widget.TextView
import kotlin.math.roundToInt

/** The command's touch target is larger than the visible label over the movie. */
internal class CaptionEdgeCommand(context: Context) : FrameLayout(context) {
    private var label = makeLabel(context)

    private fun makeLabel(localized: Context) = TextView(localized).apply {
        fun dp(value: Int) = (value * resources.displayMetrics.density).roundToInt()
        setTextSize(TypedValue.COMPLEX_UNIT_SP, 13f)
        setTextColor(Color.rgb(230, 230, 228))
        gravity = Gravity.CENTER
        maxLines = 2
        ellipsize = TextUtils.TruncateAt.END
        setPaddingRelative(dp(8), dp(1), dp(8), dp(1))
        minHeight = dp(24)
        val surface = GradientDrawable().apply {
            setColor(Color.rgb(54, 54, 56))
            cornerRadius = dp(7).toFloat()
        }
        isDuplicateParentStateEnabled = true
        background = RippleDrawable(ColorStateList.valueOf(0x22FFFFFF), surface, null)
    }

    init {
        visibility = View.GONE
        filterTouchesWhenObscured = true
        importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
        addView(label, LayoutParams(LayoutParams.WRAP_CONTENT, LayoutParams.WRAP_CONTENT, Gravity.CENTER))
    }

    fun reloadAppearance(localized: Context) {
        val position = label.layoutParams
        removeView(label)
        label = makeLabel(localized)
        addView(label, position)
    }

    fun setLabel(text: String) {
        label.text = text
    }

    /** Keep the painted gap to Mimi constant when a short label occupies less than the 48dp target. */
    @SuppressLint("RtlHardcoded")
    fun alignToPortrait(side: CaptionEdge) {
        val params = label.layoutParams as LayoutParams
        val horizontal = if (side == CaptionEdge.RIGHT) Gravity.RIGHT else Gravity.LEFT
        val gravity = horizontal or Gravity.CENTER_VERTICAL
        if (params.gravity != gravity) {
            params.gravity = gravity
            label.layoutParams = params
        }
    }

    fun measuredWidthWithin(maxWidth: Int): Int {
        val available = maxWidth.coerceAtLeast(1)
        val touchSize = (48 * label.resources.displayMetrics.density).roundToInt()
        measure(MeasureSpec.makeMeasureSpec(available, MeasureSpec.AT_MOST),
            MeasureSpec.makeMeasureSpec(touchSize, MeasureSpec.EXACTLY))
        return measuredWidth.coerceAtLeast(touchSize.coerceAtMost(available))
    }
}
