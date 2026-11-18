package app.yuxino.mimi.android.services.translation

import android.content.Context
import app.yuxino.mimi.android.provider.TextTranslationProvider
import app.yuxino.mimi.android.services.presentation.translationProviderIcon

/** Match desktop provider marks without changing their artwork, colors or aspect ratios. */
internal class TranslationProviderAdapter(context: Context, private val providers: List<TextTranslationProvider>) :
    TranslationIconAdapter(context, providers.map { context.getString(translationProviderLabel(it)) }) {
    override fun iconResource(position: Int): Int = translationProviderIcon(providers[position])
}
