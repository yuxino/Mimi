package app.yuxino.mimi.android

import android.content.Context
import android.view.MotionEvent
import android.view.View
import android.widget.FrameLayout

/** Keep the live frame intact when the appearance editor needs some of its space. */
internal class CaptionOverlayHost(context: Context) : FrameLayout(context) {
    private var previewFrameHeight: Int? = null

    fun showPreview(frameHeight: Int?) {
        if (previewFrameHeight == frameHeight) return
        previewFrameHeight = frameHeight
        importantForAccessibility = if (frameHeight != null) View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
            else View.IMPORTANT_FOR_ACCESSIBILITY_AUTO
        requestLayout()
    }

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        val fullHeight = previewFrameHeight
        if (fullHeight == null) {
            super.onMeasure(widthMeasureSpec, heightMeasureSpec)
            return
        }
        // Measure with the ordinary caption viewport, preserving chrome and
        // line breaks. Only the complete drawing scales into the remaining space.
        super.onMeasure(widthMeasureSpec, MeasureSpec.makeMeasureSpec(fullHeight, MeasureSpec.EXACTLY))
        setMeasuredDimension(measuredWidth, resolveSize(fullHeight, heightMeasureSpec))
    }

    override fun onLayout(changed: Boolean, l: Int, t: Int, r: Int, b: Int) {
        super.onLayout(changed, l, t, r, b)
        val scale = previewFrameHeight?.let { minOf(1f, height.toFloat() / it.coerceAtLeast(1)) } ?: 1f
        for (index in 0 until childCount) {
            getChildAt(index).apply {
                pivotX = 0f; pivotY = 0f
                scaleX = scale; scaleY = scale
                translationX = (this@CaptionOverlayHost.width - measuredWidth * scale) / 2
                translationY = 0f
            }
        }
    }

    // The live preview keeps updating, but none of its controls or scrolling
    // can steal a gesture from appearance editing or change the session.
    override fun onInterceptTouchEvent(event: MotionEvent): Boolean =
        previewFrameHeight != null || super.onInterceptTouchEvent(event)

    override fun onTouchEvent(event: MotionEvent): Boolean =
        previewFrameHeight != null || super.onTouchEvent(event)
}
