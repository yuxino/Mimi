package app.yuxino.mimi.android

import android.content.Context
import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.view.View
import app.yuxino.mimi.android.provider.ServiceProvider
import app.yuxino.mimi.android.provider.TextTranslationProvider

internal data class CaptionFrameStyle(
    val fontSize: Int,
    val color: Int,
    val opacity: Int,
    val backgroundAlpha: Int,
    val mode: CaptionDisplayMode,
    val alignment: CaptionAlignment,
    val dividers: Boolean = false,
) {
    companion object {
        fun saved(context: Context, originalOnly: Boolean = false) = CaptionFrameStyle(
            SettingsStore.fontSize(context), SettingsStore.translationColor(context),
            SettingsStore.overlayOpacity(context), SettingsStore.overlayBgAlpha(context),
            if (originalOnly || SettingsStore.originalTextOnly(context)) CaptionDisplayMode.ORIGINAL else SettingsStore.displayMode(context),
            SettingsStore.captionAlignment(context),
            SettingsStore.subtitleDividers(context),
        )
    }
}

/** Matches desktop subtitleBackgroundColor: pure black, independent background alpha. */
internal object CaptionSurface {
    fun apply(view: View, style: CaptionFrameStyle, immersive: Boolean) {
        view.alpha = if (immersive) 1f else style.opacity / 100f
        (view.background as GradientDrawable).setColor(Color.argb(
            if (immersive) 0 else style.backgroundAlpha.coerceIn(0, 100) * 255 / 100, 0, 0, 0,
        ))
    }
}

internal fun captionProviderMarks(context: Context): List<Int> {
    val provider = ServiceProvider.fromId(SettingsStore.provider(context))
    val translator = SettingsStore.textTranslationProvider(context)
    return buildList {
        add(serviceProviderIcon(provider))
        if (SettingsStore.configuration(context).usesIndependentTextTranslation && translator != TextTranslationProvider.BUILTIN && translator != TextTranslationProvider.NONE) {
            add(translationProviderIcon(translator))
        }
    }.distinct()
}

/** Native frame sizing shared by the preview and WindowManager adapter. */
internal fun captionPanelHeight(availableHeight: Int, percent: Int, chromeHeight: Int, minimumReadingHeight: Int, limit: Int = availableHeight): Int {
    return maxOf(chromeHeight + minimumReadingHeight, availableHeight * percent.coerceIn(10, 70) / 100).coerceAtMost(limit)
}
