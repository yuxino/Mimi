package app.yuxino.mimi.android.shortcuts

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

/** A separate, transient touch target. When hidden, the underlying player gets this space back. */
internal class CaptionEdgeCommand(context: Context) : FrameLayout(context) {
    private fun dp(value: Int) = (value * resources.displayMetrics.density).roundToInt()
    private val label = TextView(context).apply {
        setTextSize(TypedValue.COMPLEX_UNIT_SP, 14f)
        setTextColor(Color.rgb(47, 39, 46))
        gravity = Gravity.CENTER
        maxLines = 2
        ellipsize = TextUtils.TruncateAt.END
        setPadding(dp(10), dp(3), dp(10), dp(3))
        minHeight = dp(28)
        background = GradientDrawable().apply {
            setColor(Color.rgb(253, 249, 251))
            cornerRadius = dp(9).toFloat()
            setStroke(dp(1), Color.rgb(199, 187, 195))
        }
        isDuplicateParentStateEnabled = true
        foreground = RippleDrawable(ColorStateList.valueOf(0x22352B33), null, background.constantState?.newDrawable())
    }

    init {
        visibility = View.GONE
        filterTouchesWhenObscured = true
        // TalkBack already operates the same action through the portrait button.
        importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
        addView(label, LayoutParams(LayoutParams.WRAP_CONTENT, LayoutParams.WRAP_CONTENT, Gravity.CENTER))
    }

    fun setLabel(value: String) { label.text = value }

    fun measuredWidthWithin(maxWidth: Int): Int {
        measure(MeasureSpec.makeMeasureSpec(maxWidth.coerceAtLeast(1), MeasureSpec.AT_MOST),
            MeasureSpec.makeMeasureSpec(dp(48), MeasureSpec.EXACTLY))
        return measuredWidth.coerceAtLeast(dp(48).coerceAtMost(maxWidth.coerceAtLeast(1)))
    }
}
