package app.yuxino.mimi.android.captions

import app.yuxino.mimi.android.R

/** View-only filtering. Recognition and shared paired subtitles remain unchanged. */
enum class CaptionDisplayMode(val id: String, val label: Int, val sourceVisible: Boolean, val translationVisible: Boolean) {
    TRANSLATION("translation", R.string.display_translation, false, true),
    ORIGINAL("original", R.string.display_original, true, false),
    BILINGUAL("bilingual", R.string.display_bilingual, true, true);

    companion object {
        fun fromId(id: String?): CaptionDisplayMode = entries.firstOrNull { it.id == id } ?: BILINGUAL
    }
}
