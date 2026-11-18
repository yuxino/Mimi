package app.yuxino.mimi.android.captions

import android.view.Gravity
import app.yuxino.mimi.android.R

/** Text presentation only; never changes recognition or paired subtitles. */
enum class CaptionAlignment(val id: String, val label: Int, val gravity: Int) {
    LEFT("left", R.string.settings_align_left, Gravity.START),
    CENTER("center", R.string.settings_align_center, Gravity.CENTER_HORIZONTAL),
    RIGHT("right", R.string.settings_align_right, Gravity.END);

    companion object {
        fun fromId(id: String?): CaptionAlignment = entries.firstOrNull { it.id == id } ?: CENTER
    }
}
