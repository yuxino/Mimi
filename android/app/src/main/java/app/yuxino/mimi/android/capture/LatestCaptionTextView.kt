package app.yuxino.mimi.android.capture

import android.content.Context
import android.annotation.SuppressLint
import android.view.ViewTreeObserver
import android.widget.TextView

/** A bounded compact viewport following the latest lines without discarding text. */
// This service-owned system overlay has no AppCompat Activity/theme.
@SuppressLint("AppCompatCustomView")
internal class LatestCaptionTextView(context: Context) : TextView(context) {
    private val followLatest = ViewTreeObserver.OnPreDrawListener {
        // Text can relayout without changing this view's size. Wait for that
        // layout too, and redraw once if the viewport moved before painting.
        !followTail()
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        viewTreeObserver.addOnPreDrawListener(followLatest)
    }

    override fun onDetachedFromWindow() {
        viewTreeObserver.removeOnPreDrawListener(followLatest)
        super.onDetachedFromWindow()
    }

    override fun onLayout(changed: Boolean, left: Int, top: Int, right: Int, bottom: Int) {
        super.onLayout(changed, left, top, right, bottom)
        followTail()
    }

    private fun followTail(): Boolean {
        val textLayout = layout ?: return false
        val visibleHeight = height - compoundPaddingTop - compoundPaddingBottom
        if (visibleHeight <= 0) return false
        // Align a whole line, rather than cutting through the preceding line's
        // glyphs because the first line includes extra font padding.
        val firstVisibleLine = (textLayout.lineCount - maxLines).coerceAtLeast(0)
        val tail = textLayout.getLineTop(firstVisibleLine)
        if (scrollY == tail) return false
        scrollTo(0, tail)
        return true
    }
}
