package app.yuxino.mimi.android

import android.content.Context
import android.text.Layout
import android.util.AttributeSet
import android.widget.LinearLayout
import android.widget.TextView

/** Use one footer row when the actual localized labels fit, otherwise stack them. */
class SessionControlsLayout @JvmOverloads constructor(context: Context, attrs: AttributeSet? = null) :
    LinearLayout(context, attrs) {
    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        val state = getChildAt(0)
        val action = getChildAt(1)
        action.measure(MeasureSpec.makeMeasureSpec(0, MeasureSpec.UNSPECIFIED),
            MeasureSpec.makeMeasureSpec(0, MeasureSpec.UNSPECIFIED))
        val label = findViewById<TextView>(R.id.status)
        val stateWidth = Layout.getDesiredWidth(label.text, label.paint) + dp(48)
        val available = MeasureSpec.getSize(widthMeasureSpec) - paddingLeft - paddingRight
        val fits = stateWidth + action.measuredWidth + dp(16) <= available
        orientation = if (fits) HORIZONTAL else VERTICAL
        gravity = android.view.Gravity.CENTER_VERTICAL
        val stateParams = state.layoutParams as LayoutParams
        stateParams.width = if (fits) 0 else LayoutParams.MATCH_PARENT
        stateParams.height = LayoutParams.WRAP_CONTENT
        stateParams.weight = if (fits) 1f else 0f
        stateParams.marginEnd = if (fits) dp(16) else 0
        stateParams.bottomMargin = if (fits) 0 else dp(8)
        stateParams.resolveLayoutDirection(layoutDirection)
        val actionParams = action.layoutParams as LayoutParams
        actionParams.width = if (fits) LayoutParams.WRAP_CONTENT else LayoutParams.MATCH_PARENT
        actionParams.height = LayoutParams.WRAP_CONTENT
        super.onMeasure(widthMeasureSpec, heightMeasureSpec)
    }
    private fun dp(value: Int) = (value * resources.displayMetrics.density + .5f).toInt()
}
