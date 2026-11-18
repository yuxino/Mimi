package app.yuxino.mimi.android.services.translation

import android.content.Context
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.services.presentation.translationPresetIcon
import app.yuxino.mimi.android.services.setup.TextTranslationPresets

internal class TextTranslationPresetAdapter(context: Context) : TranslationIconAdapter(context,
    listOf(context.getString(R.string.translation_preset_custom)) + TextTranslationPresets.entries.map { it.label }) {
    override fun iconResource(position: Int): Int = translationPresetIcon(TextTranslationPresets.entries.getOrNull(position - 1)?.id)
}
