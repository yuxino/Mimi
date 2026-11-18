package app.yuxino.mimi.android.ui.layout

import android.app.Activity
import android.view.ViewGroup
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import app.yuxino.mimi.android.R

/** Android 15 enforces edge-to-edge; keep controls clear of bars and cutouts. */
internal fun Activity.applySystemBarInsets() {
    val root = findViewById<ViewGroup>(android.R.id.content).getChildAt(0)
    val left = root.paddingLeft
    val top = root.paddingTop
    val right = root.paddingRight
    val bottom = root.paddingBottom
    ViewCompat.setOnApplyWindowInsetsListener(root) { view, insets ->
        val bars = insets.getInsets(
            WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout(),
        )
        view.setPadding(left + bars.left, top + bars.top, right + bars.right, bottom + maxOf(bars.bottom, insets.getInsets(WindowInsetsCompat.Type.ime()).bottom))
        insets
    }
    ViewCompat.requestApplyInsets(root)
}
