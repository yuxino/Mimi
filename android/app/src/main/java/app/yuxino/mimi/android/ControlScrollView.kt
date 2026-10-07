package app.yuxino.mimi.android

import android.content.Context
import android.graphics.Rect
import android.util.AttributeSet
import android.view.InputDevice
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.widget.ScrollView

/** Keep the framework's enlarged scrollbar hover target from swallowing nearby controls. */
class ControlScrollView @JvmOverloads constructor(context: Context, attrs: AttributeSet? = null) : ScrollView(context, attrs) {
    override fun onInterceptHoverEvent(event: MotionEvent): Boolean {
        if (!super.onInterceptHoverEvent(event)) return false
        if (event.isFromSource(InputDevice.SOURCE_MOUSE) &&
            (event.actionMasked == MotionEvent.ACTION_HOVER_ENTER || event.actionMasked == MotionEvent.ACTION_HOVER_MOVE)) {
            val origin = IntArray(2).also(rootView::getLocationOnScreen)
            if (hasControlAt(this, event.rawX.toInt() - origin[0], event.rawY.toInt() - origin[1])) return false
        }
        return true
    }

    private fun hasControlAt(view: View, x: Int, y: Int): Boolean {
        if (!view.isShown || !view.isEnabled) return false
        val visible = Rect()
        if (!view.getGlobalVisibleRect(visible) || !visible.contains(x, y)) return false
        if (view !== this && (view.isClickable || view.isLongClickable)) return true
        return view is ViewGroup && (0 until view.childCount).any { hasControlAt(view.getChildAt(it), x, y) }
    }
}
