package app.yuxino.mimi.android.shortcuts

import android.graphics.Rect
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.view.inspector.WindowInspector
import android.widget.TextView

/** Find only the app's actual overlay roots, even when the activity beneath is another app. */
internal object CaptionShortcutWindows {
    private fun overlays() = WindowInspector.getGlobalWindowViews().filter {
        it.isShown && (it.layoutParams as? WindowManager.LayoutParams)?.type == WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY
    }

    fun character(description: String, size: Int) = overlays().singleOrNull {
        it.contentDescription == description && it.width == size && it.height == size
    }

    fun command(label: String): View? {
        fun containsLabel(view: View): Boolean = view.isShown && when (view) {
            is TextView -> view.text.toString() == label
            is ViewGroup -> (0 until view.childCount).any { containsLabel(view.getChildAt(it)) }
            else -> false
        }
        return overlays().singleOrNull(::containsLabel)
    }

    fun bounds(view: View): Rect {
        val xy = IntArray(2).also(view::getLocationOnScreen)
        return Rect(xy[0], xy[1], xy[0] + view.width, xy[1] + view.height)
    }
}
