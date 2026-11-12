package app.yuxino.mimi.android

import android.content.Context
import android.util.AttributeSet
import android.view.View
import android.widget.LinearLayout

/** Keep the single caption preview in view while its settings scroll independently. */
class CaptionWorkspace @JvmOverloads constructor(context: Context, attrs: AttributeSet? = null) :
    LinearLayout(context, attrs) {
    var preferredHeight: Int? = null
    var showsPreview = true
        set(value) { if (field != value) { field = value; requestLayout() } }

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        val preview = getChildAt(0)
        val controls = getChildAt(1)
        preview.visibility = if (showsPreview) View.VISIBLE else View.GONE
        val width = MeasureSpec.getSize(widthMeasureSpec)
        val availableHeight = MeasureSpec.getSize(heightMeasureSpec)
        val height = preferredHeight?.let {
            if (MeasureSpec.getMode(heightMeasureSpec) == MeasureSpec.UNSPECIFIED) it else it.coerceAtMost(availableHeight)
        } ?: availableHeight
        val sideBySide = showsPreview && width >= dp(480) && width > height
        orientation = if (sideBySide) HORIZONTAL else VERTICAL
        preview.findViewById<SubtitlePreviewView>(R.id.subtitle_preview)?.limitHeight(
            if (sideBySide) height else (height - dp(128)).coerceAtLeast(0))
        val previewParams = preview.layoutParams as LayoutParams
        val controlsParams = controls.layoutParams as LayoutParams
        // Measurement already owns layout; reuse parameters without requesting another pass.
        if (sideBySide) {
            previewParams.width = (width * .4f).toInt().coerceAtMost(dp(400))
            previewParams.height = LayoutParams.MATCH_PARENT
            controlsParams.width = 0
            controlsParams.height = LayoutParams.MATCH_PARENT
        } else {
            preview.measure(MeasureSpec.makeMeasureSpec(width, MeasureSpec.EXACTLY),
                MeasureSpec.makeMeasureSpec((height - dp(128)).coerceAtLeast(0), MeasureSpec.AT_MOST))
            previewParams.width = LayoutParams.MATCH_PARENT
            previewParams.height = if (showsPreview) preview.measuredHeight else 0
            controlsParams.width = LayoutParams.MATCH_PARENT
            controlsParams.height = 0
        }
        previewParams.weight = 0f
        controlsParams.weight = 1f
        super.onMeasure(widthMeasureSpec, MeasureSpec.makeMeasureSpec(height, MeasureSpec.EXACTLY))
    }

    private fun dp(value: Int) = (value * resources.displayMetrics.density + .5f).toInt()
}
