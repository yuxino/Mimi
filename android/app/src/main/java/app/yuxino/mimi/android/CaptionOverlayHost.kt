package app.yuxino.mimi.android

import android.content.Context
import android.widget.FrameLayout

internal class CaptionOverlayHost(context: Context) : FrameLayout(context) {
    override fun onLayout(changed: Boolean, l: Int, t: Int, r: Int, b: Int) {
        super.onLayout(changed, l, t, r, b)
        val scale = 1f
        for (index in 0 until childCount) {
            getChildAt(index).apply {
                pivotX = 0f; pivotY = 0f
                scaleX = scale; scaleY = scale
                translationX = (this@CaptionOverlayHost.width - measuredWidth * scale) / 2
                translationY = 0f
            }
        }
    }
}
