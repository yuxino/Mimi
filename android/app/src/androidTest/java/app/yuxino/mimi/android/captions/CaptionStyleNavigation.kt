package app.yuxino.mimi.android.captions

import android.view.View
import android.widget.RadioGroup
import app.yuxino.mimi.android.MainActivity
import app.yuxino.mimi.android.R

/** Native controls are directly available beside the single pinned preview. */
internal fun openCaptionStyles(activity: MainActivity) {
    activity.findViewById<View>(R.id.tab_caption).performClick()
    check(activity.findViewById<RadioGroup>(R.id.settings_tabs).checkedRadioButtonId == R.id.tab_caption)
    val font = activity.findViewById<View>(R.id.font_size)
    font.requestRectangleOnScreen(android.graphics.Rect(0, 0, font.width, font.height), true)
}
