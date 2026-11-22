package app.yuxino.mimi.android.shortcuts

import android.content.Context
import android.content.res.ColorStateList
import android.graphics.Color
import android.graphics.Typeface
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
    private val ink = Color.rgb(248, 248, 246)
    private val label = TextView(context).apply {
        setTextSize(TypedValue.COMPLEX_UNIT_SP, 13f)
        setTextColor(ink)
        typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL)
        gravity = Gravity.CENTER
        maxLines = 2
        ellipsize = TextUtils.TruncateAt.END
        compoundDrawablePadding = dp(3)
        setPaddingRelative(dp(8), dp(1), dp(10), dp(1))
        minHeight = dp(26)
        background = GradientDrawable().apply {
            setColor(Color.argb(239, 37, 37, 39))
            cornerRadius = dp(24).toFloat()
            setStroke(1, Color.argb(60, 255, 255, 255))
        }
        isDuplicateParentStateEnabled = true
        foreground = RippleDrawable(ColorStateList.valueOf(0x33FFFFFF), null, background.constantState?.newDrawable())
    }

    init {
        visibility = View.GONE
        filterTouchesWhenObscured = true
        // TalkBack already operates the same action through the portrait button.
        importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
        addView(label, LayoutParams(LayoutParams.WRAP_CONTENT, LayoutParams.WRAP_CONTENT, Gravity.CENTER))
    }

    fun setAction(text: String, icon: Int?) {
        label.text = text
        val symbol = icon?.let { context.getDrawable(it)?.mutate() }?.apply {
            setTint(ink)
            setBounds(0, 0, dp(16), dp(16))
        }
        label.setCompoundDrawablesRelative(symbol, null, null, null)
    }

    fun measuredWidthWithin(maxWidth: Int): Int {
        measure(MeasureSpec.makeMeasureSpec(maxWidth.coerceAtLeast(1), MeasureSpec.AT_MOST),
            MeasureSpec.makeMeasureSpec(dp(48), MeasureSpec.EXACTLY))
        return measuredWidth.coerceAtLeast(dp(48).coerceAtMost(maxWidth.coerceAtLeast(1)))
    }
}
